import { render, act, screen } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  ConfigProvider,
  ConfigProviderForTest,
  DEFAULT_CONFIG,
  useConfig,
  type AppConfig,
} from '../ConfigContext';
import {
  invoke,
  listen,
  emitTauriEvent,
  clearEventHandlers,
} from '../../testUtils/mocks/tauri';

function Probe() {
  const config = useConfig();
  return (
    <>
      <div data-testid="ollama-url">{config.inference.ollamaUrl}</div>
      <div data-testid="overlay-width">{config.window.overlayWidth}</div>
      <div data-testid="max-chat-height">{config.window.maxChatHeight}</div>
      <div data-testid="max-display-lines">{config.quote.maxDisplayLines}</div>
      <div data-testid="system-prompt">{config.prompt.system}</div>
    </>
  );
}

describe('ConfigContext', () => {
  beforeEach(() => {
    invoke.mockReset();
    listen.mockClear();
    clearEventHandlers();
  });

  describe('useConfig fallback', () => {
    it('returns DEFAULT_CONFIG when no provider is in the tree', () => {
      render(<Probe />);
      expect(screen.getByTestId('ollama-url').textContent).toBe(
        DEFAULT_CONFIG.inference.ollamaUrl,
      );
      expect(screen.getByTestId('overlay-width').textContent).toBe(
        String(DEFAULT_CONFIG.window.overlayWidth),
      );
      expect(screen.getByTestId('max-display-lines').textContent).toBe(
        String(DEFAULT_CONFIG.quote.maxDisplayLines),
      );
    });
  });

  describe('ConfigProviderForTest', () => {
    it('provides the supplied value to descendants', () => {
      const custom: AppConfig = {
        ...DEFAULT_CONFIG,
        inference: {
          ollamaUrl: 'http://example.test:11434',
        },
      };
      render(
        <ConfigProviderForTest value={custom}>
          <Probe />
        </ConfigProviderForTest>,
      );
      expect(screen.getByTestId('ollama-url').textContent).toBe(
        'http://example.test:11434',
      );
    });
  });

  describe('ConfigProvider', () => {
    it('hydrates from the backend and transforms snake_case to camelCase', async () => {
      invoke.mockResolvedValueOnce({
        inference: {
          ollama_url: 'http://127.0.0.1:11434',
        },
        prompt: { system: 'custom base prompt' },
        window: {
          overlay_width: 800,
          max_chat_height: 700,
        },
        quote: {
          max_display_lines: 6,
          max_display_chars: 500,
          max_context_length: 8192,
        },
      });

      render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      // Let the useEffect + promise resolution flush.
      await act(async () => {});

      expect(screen.getByTestId('ollama-url').textContent).toBe(
        'http://127.0.0.1:11434',
      );
      expect(screen.getByTestId('overlay-width').textContent).toBe('800');
      expect(screen.getByTestId('max-chat-height').textContent).toBe('700');
      expect(screen.getByTestId('max-display-lines').textContent).toBe('6');
      expect(screen.getByTestId('system-prompt').textContent).toBe(
        'custom base prompt',
      );
    });

    it('falls back to DEFAULT_CONFIG when invoke returns nullish', async () => {
      invoke.mockResolvedValueOnce(undefined);

      render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      await act(async () => {});

      expect(screen.getByTestId('ollama-url').textContent).toBe(
        DEFAULT_CONFIG.inference.ollamaUrl,
      );
      expect(screen.getByTestId('overlay-width').textContent).toBe(
        String(DEFAULT_CONFIG.window.overlayWidth),
      );
    });

    it('falls back to DEFAULT_CONFIG when invoke rejects', async () => {
      invoke.mockRejectedValueOnce(new Error('IPC bridge unavailable'));

      render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      await act(async () => {});

      expect(screen.getByTestId('ollama-url').textContent).toBe(
        DEFAULT_CONFIG.inference.ollamaUrl,
      );
      expect(screen.getByTestId('overlay-width').textContent).toBe(
        String(DEFAULT_CONFIG.window.overlayWidth),
      );
    });

    it('renders nothing before the initial invoke resolves', () => {
      invoke.mockImplementation(
        () => new Promise<never>(() => {}), // pending forever
      );
      const { container } = render(
        <ConfigProvider>
          <div data-testid="child">child</div>
        </ConfigProvider>,
      );
      expect(container.textContent).toBe('');
    });

    it('refetches and updates state when thuki://config-updated fires', async () => {
      const initial = {
        inference: { ollama_url: 'http://127.0.0.1:11434' },
        prompt: { system: '' },
        window: {
          overlay_width: 600,
          max_chat_height: 648,
        },
        quote: {
          max_display_lines: 4,
          max_display_chars: 300,
          max_context_length: 4096,
        },
      };
      const updated = {
        ...initial,
        window: {
          overlay_width: 900,
          max_chat_height: 800,
        },
      };
      invoke.mockResolvedValueOnce(initial).mockResolvedValueOnce(updated);

      render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      await act(async () => {});
      expect(screen.getByTestId('overlay-width').textContent).toBe('600');

      await act(async () => {
        emitTauriEvent('thuki://config-updated', null);
      });

      expect(screen.getByTestId('overlay-width').textContent).toBe('900');
      expect(screen.getByTestId('max-chat-height').textContent).toBe('800');
    });

    it('keeps last good config when a refresh invoke rejects', async () => {
      const initial = {
        inference: { ollama_url: 'http://127.0.0.1:11434' },
        prompt: { system: 'p' },
        window: {
          overlay_width: 700,
          max_chat_height: 648,
        },
        quote: {
          max_display_lines: 4,
          max_display_chars: 300,
          max_context_length: 4096,
        },
      };
      invoke
        .mockResolvedValueOnce(initial)
        .mockRejectedValueOnce(new Error('transient'))
        .mockResolvedValueOnce(undefined);

      render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      await act(async () => {});
      expect(screen.getByTestId('overlay-width').textContent).toBe('700');

      // Rejected refresh: state stays at the last good value (no flip to defaults).
      await act(async () => {
        emitTauriEvent('thuki://config-updated', null);
      });
      expect(screen.getByTestId('overlay-width').textContent).toBe('700');

      // Nullish refresh: same — state preserved.
      await act(async () => {
        emitTauriEvent('thuki://config-updated', null);
      });
      expect(screen.getByTestId('overlay-width').textContent).toBe('700');
    });

    it('unsubscribes on unmount', async () => {
      invoke.mockResolvedValue({
        inference: { ollama_url: 'http://127.0.0.1:11434' },
        prompt: { system: '' },
        window: {
          overlay_width: 600,
          max_chat_height: 648,
        },
        quote: {
          max_display_lines: 4,
          max_display_chars: 300,
          max_context_length: 4096,
        },
      });

      const { unmount } = render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      await act(async () => {});
      const callsBeforeUnmount = invoke.mock.calls.length;
      unmount();
      await act(async () => {
        emitTauriEvent('thuki://config-updated', null);
      });
      expect(invoke.mock.calls.length).toBe(callsBeforeUnmount);
    });

    it('survives a listen() rejection without crashing initial hydrate', async () => {
      listen.mockRejectedValueOnce(new Error('event bridge missing'));
      invoke.mockResolvedValueOnce({
        inference: { ollama_url: 'http://127.0.0.1:11434' },
        prompt: { system: '' },
        window: {
          overlay_width: 600,
          max_chat_height: 648,
        },
        quote: {
          max_display_lines: 4,
          max_display_chars: 300,
          max_context_length: 4096,
        },
      });

      render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      await act(async () => {});
      expect(screen.getByTestId('overlay-width').textContent).toBe('600');
    });

    it('ignores a late-resolving invoke after unmount', async () => {
      let resolveInvoke: ((raw: unknown) => void) | undefined;
      invoke.mockImplementationOnce(
        () =>
          new Promise<unknown>((resolve) => {
            resolveInvoke = resolve;
          }),
      );

      const { unmount } = render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      unmount();
      // Resolve after unmount: the cancelled-guard short-circuits the setState.
      // No assertion on output (provider gone); the run is the coverage signal.
      await act(async () => {
        resolveInvoke!({
          inference: { ollama_url: 'http://127.0.0.1:11434' },
          prompt: { system: '' },
          window: {
            overlay_width: 600,
            max_chat_height: 648,
          },
          quote: {
            max_display_lines: 4,
            max_display_chars: 300,
            max_context_length: 4096,
          },
        });
      });
    });

    it('ignores a late-rejecting invoke after unmount', async () => {
      let rejectInvoke: ((err: unknown) => void) | undefined;
      invoke.mockImplementationOnce(
        () =>
          new Promise<unknown>((_resolve, reject) => {
            rejectInvoke = reject;
          }),
      );

      const { unmount } = render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      unmount();
      await act(async () => {
        rejectInvoke!(new Error('late'));
      });
    });

    it('drops a late-arriving listen subscription if already unmounted', async () => {
      let resolveListen: ((fn: () => void) => void) | undefined;
      const unlistenSpy = vi.fn();
      listen.mockImplementationOnce(
        () =>
          new Promise<() => void>((resolve) => {
            resolveListen = resolve;
          }),
      );
      invoke.mockResolvedValueOnce({
        inference: { ollama_url: 'http://127.0.0.1:11434' },
        prompt: { system: '' },
        window: {
          overlay_width: 600,
          max_chat_height: 648,
        },
        quote: {
          max_display_lines: 4,
          max_display_chars: 300,
          max_context_length: 4096,
        },
      });

      const { unmount } = render(
        <ConfigProvider>
          <Probe />
        </ConfigProvider>,
      );
      await act(async () => {});
      unmount();
      await act(async () => {
        resolveListen!(unlistenSpy);
      });
      expect(unlistenSpy).toHaveBeenCalledTimes(1);
    });
  });
});
