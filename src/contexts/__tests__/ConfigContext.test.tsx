import { render, act, screen } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
import {
  ConfigProvider,
  ConfigProviderForTest,
  DEFAULT_CONFIG,
  useConfig,
  type AppConfig,
} from '../ConfigContext';
import { invoke } from '../../testUtils/mocks/tauri';

function Probe() {
  const config = useConfig();
  return (
    <>
      <div data-testid="ollama-url">{config.inference.ollamaUrl}</div>
      <div data-testid="overlay-width">{config.window.overlayWidth}</div>
      <div data-testid="max-display-lines">{config.quote.maxDisplayLines}</div>
      <div data-testid="system-prompt">{config.prompt.system}</div>
      <div data-testid="hide-delay">{config.window.hideCommitDelayMs}</div>
    </>
  );
}

describe('ConfigContext', () => {
  beforeEach(() => {
    invoke.mockReset();
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
          collapsed_height: 100,
          max_chat_height: 700,
          hide_commit_delay_ms: 400,
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
      expect(screen.getByTestId('max-display-lines').textContent).toBe('6');
      expect(screen.getByTestId('system-prompt').textContent).toBe(
        'custom base prompt',
      );
      expect(screen.getByTestId('hide-delay').textContent).toBe('400');
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
  });
});
